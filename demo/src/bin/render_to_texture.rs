//! # Dokuya çizim ve iki geçiş
//!
//! Sahneyi pencereye değil bir **dokuya** çizmek, ve aynı kareyi birden fazla kameradan çizmek.
//!
//! ## 2026-11-10: kamera kendi dokusuna çiziyor
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | bir kamerayı pencere yerine bir dokuya çizmek | **var** — `CameraTarget`, kameranın bileşeni |
//! | aynı karede iki kameradan iki görüntü | **var** — motor hedefli her kamerayı kendi gönderiminde çiziyor |
//! | her kameranın kendi gölge kaskadları ve kümeleri | **var** (2026-08-24) |
//! | her kameranın kendi TAA/SSGI geçmişi | **var** — hedefli kamera kendi geçmiş yuvasında |
//! | hedefin bir alt dikdörtgenine çizmek (alt-görüş) | **var** — `Camera::viewport` (bölünmüş ekran, 2026-11-01) |
//!
//! Tepedeki kameranın bir `CameraTarget`'i var: 512×288'lik, motorun çıkış biçiminde bir doku. Motor
//! onu her kare, karenin kendi görüntüsünden önce, kendi kodlayıcısında çizip gönderiyor; kare de
//! öndeki kameradan (`primary`) pencereye çiziliyor. Oyun kodunda kamera seçimi yok, ikinci bir
//! çizim çağrısı yok, ve ikisi de oyun dünyasının kopyasından, render thread'inde çiziliyor. Doku
//! örneklenebiliyor ve kopyalanabiliyor: demo onu 60. karede PNG olarak yazıyor (`GIZMO_RTT_DUMP`),
//! karenin üstüne çizilen bir geçişten (`RenderWorld::with_pass`), kopyadaki kameranın hedefinden.
//!
//! Bu, aşağıdaki üç engelin varılan yeri. Önceki hâli bir render kancasıydı: `primary` bayrağını
//! iki `default_render_pass` çağrısı arasında çeviriyordu — kareyi **iki kez hazırlayarak** (her
//! animasyon ve parçacık iki kat hızlı) ve render thread'inden vazgeçerek. Aşağısı o yolun tarihi.
//!
//! ## Tarih: ikinci kameraya giden üç engel
//!
//! Hedef seçimi baştan çalışıyordu, çünkü çizim işlevinin imzası hedefi dışarıdan alıyor:
//!
//! ```text
//! default_render_pass(world, encoder, view, renderer)
//! ```
//!
//! İkinci kameradan ikinci bir geçiş üç ayrı engelle karşılaştı, **üçü de ölçüldü**, ve biri
//! bu demo sayesinde motorda kapandı.
//!
//! ### Engel 1: çizim işlevi kamera parametresi almıyor
//!
//! Seçimi `active_camera` yapıyor: "işaretli olan, yoksa ilki". Yani kamerayı değiştirmenin tek
//! yolu geçişler arasında `primary` bayrağını çevirmek. **Bu kısım çalışıyor** — ölçüldü:
//!
//! ```text
//! 1. geçiş (çevrim dışı): active_camera=Some(5)
//! 2. geçiş (pencere):     active_camera=Some(4)
//! ```
//!
//! ### Engel 2: basit sahne HER kameraya tek bir duruş damgalıyordu — **kapatıldı**
//!
//! Bu demo yazılırken bulundu ve motorda düzeltildi (2026-08-23). Basit sahnenin güncelleme
//! döngüsü duruşu `(Transform, Camera)` eşleşen **her** varlığa yazıyordu, hiçbir ayrım
//! gözetmeden:
//!
//! ```text
//! for (mut trans, mut cam) in q.iter_mut() {   // filtre yok
//! ```
//!
//! Ölçülen sonuç: ikinci kamera (0 · 7,0 · 0,2) konumunda, pitch −1,15 ile doğuruldu; çizim
//! anında okunan değer **`yaw −1,57 pitch −0,21 T(0 · 1,4 · 6,5)`** — yani birincisinin duruşu.
//! İkinci bir kamera kendi yerinde duramıyordu.
//!
//! Artık duruş yalnız `active_camera`'nın seçtiği kameraya yazılıyor — yani renderer'ın kendi
//! kuralına, ikinci bir kopyasına değil. Böylece "oyuncunun uçurduğu kamera" ile "karenin
//! çizildiği kamera" aynı cümle oluyor. `gizmo::simple::simple_scene_update` artık dışa açık ve
//! bir regresyon testi ile kilitli (`the_fly_camera_writes_only_to_the_rendered_camera` —
//! filtreyi geri alınca kırmızıya dönüyor).
//!
//! Demonun taşıdığı geçici çözüm bu yüzden **kaldırıldı**: ikinci kamera artık hiçbir şey
//! yapmadan kendi yerinde duruyor.
//!
//! ### Engel 3 (asıl olan): tek bir uniform tamponu vardı — **kapatıldı**
//!
//! Duruş geri yazıldıktan sonra bile iki geçiş **aynı görüntüyü** verdi. Sebep boru hattının
//! kendisindeydi: kamera matrisleri **tek** bir `global_uniform_buffer`'a
//! `renderer.queue.write_buffer(...)` ile yazılıyordu. `write_buffer` yazımları encoder'a kaydetme
//! anına göre değil **gönderime** göre sıralanıyor — yani iki geçiş aynı encoder'a kaydedilirse
//! ikisi de son yazılan kamerayı okur.
//!
//! O zamanki çare birinci geçişi **kendi encoder'ına** kaydedip hemen `submit` etmekti: kamera
//! başına bir gönderim, belgelenmemiş, ve bir düzlemsel aynanın ya da yansıma probunun istediği
//! kaç görünüm varsa o kadar gönderim demek.
//!
//! **2026-08-23: `SceneView`.** Kendi uniform tamponu ve kendi grup-0 bağlama grubu olan bir
//! görünüm. İki yazımın çarpışacağı yer kalmıyor, o yüzden tek encoder ve tek gönderim yetiyor:
//!
//! ```ignore
//! renderer.scene.views.push(SceneView::new(&renderer.device, &renderer.scene, "ayna"));
//! renderer.scene.active_view = Some(0);
//! default_render_pass(world, encoder, &hedef, renderer);   // 1. kamera
//! renderer.scene.active_view = None;
//! default_render_pass(world, encoder, pencere, renderer);  // 2. kamera, aynı encoder
//! ```
//!
//! Küme tablosu ve ışık dizini paylaşılıyor, çoğaltılmıyor: onlar sahneden türetiliyor, kameradan
//! değil.
//!
//! ### Ölçüldü — ve sınırı da kapandı (2026-08-24)
//!
//! Çevrim dışı hedefin parlaklık profili (2026-08-23, 512×288, kare 90):
//!
//! | kip | üst %0-20 | orta %40-60 | alt %80-100 |
//! |-----|-----------|-------------|-------------|
//! | tek encoder (tuzak) | 54,42 | 139,19 | 84,40 |
//! | ayrı encoder + hemen submit | 88,80 | 101,39 | 63,07 |
//! | **`SceneView`, tek encoder** | **88,80** | **101,46** | **62,82** |
//!
//! Üç bant da kamera başına gönderim çözümüyle uyuşuyor: üst bant birebir, orta 0,07, alt 0,25.
//!
//! ### Ölçüm notu: bu tablo iki aşamada oluştu
//!
//! `SceneView` ilk hâliyle yalnız **kamera uniform'unu** ayırıyordu, ve o zaman alt bant 86,14
//! okuyordu — ayrı-encoder çözümünden **23 puan** uzak. Sebebini tahmin etmek yerine ölçtüm:
//! `GIZMO_RTT_NO_SHADOW=1` güneşi gölgesiz bir dolgu ışığına çevirince üç bant da 1,5 puanın
//! altında uyuşuyordu, yani farkın tamamı gölgeden geliyordu.
//!
//! Sorumlu, kameradan **türetilen** ikinci bir uniform ailesiydi: kaskad ışık-görüş matrisleri ve
//! küme tablosu. İkisi de `queue.write_buffer` ile yazılıyor, yani engel 3'ün birebir aynısı bir
//! seviye aşağıda. 2026-08-24'te ikisi de `SceneView`'a taşındı ve alt bant 62,82'ye indi.
//!
//! Gölge **dokusunun** kopyalanması gerekmedi, ve bu ayrı bir ölçüm: render geçişleri kaydedilme
//! sırasına göre koşuyor, yani her görünümün gölge geçişi kendi ana geçişinden hemen önce aynı
//! dokuyu yeniden çiziyor. Kopyalanması gereken tek şey uniform'lardı. Görünüm başına maliyet
//! **~460 KB**, ikinci bir 3072²×4 kaskad dizisinin gerektireceği **144 MB** değil.
//!
//! ## Sonuç (2026-08-24)
//!
//! Dokuya çizim **çalışıyordu**, iki geçiş **tek encoder'da mümkündü**, ve `primary` çevirmek dışında
//! bir geçici çözüm kalmamıştı. Kalan paylaşılan şey **zamansal** durumdu: TAA geçmişi ve SSGI
//! birikimi görünüm başına değildi. Motorun `two_cameras_in_one_encoder_render_two_different_frames`
//! testi o yolu kilitliyor: görünümsüz iki geçiş **birebir aynı** kareyi vermeli (tuzak hâlâ orada),
//! görünümlü iki geçiş farklı kareler vermeli. 2026-11-10'dan beri ne `primary` çevirmek ne ikinci
//! geçiş gerekiyor: hedefli kamera kendi gönderiminde, kendi geçmişiyle çiziliyor
//! (`a_camera_with_a_target_draws_into_it_and_leaves_the_frame_alone`: hedefteki resim, aynı kameranın
//! karenin kendi kamerası olarak çizdiği resimle piksel piksel aynı; karenin kendi görüntüsü onun
//! varlığından etkilenmiyor).
//!
//! ## Kontroller
//!   * `GIZMO_RTT_DUMP=<dizin>` — tepedeki kameranın dokusunu 60. karede PNG olarak yaz
//!   * `GIZMO_RTT_NO_SHADOW=1` — güneşi gölgesiz dolgu ışığına çevir
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::renderer::components::CameraTarget;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Sahnedeki iki kameradan hangisi olduğunu işaretler.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CameraTag(u8);
gizmo::core::impl_component!(CameraTag);

/// Arayüzün gösterdiği.
#[derive(Default, Clone)]
struct RttReport {
    frame: u32,
    size: (u32, u32),
}
gizmo::core::impl_component!(RttReport);

const OFF_W: u32 = 512;
const OFF_H: u32 = 288;

/// Tepedeki kameranın duruşu. Basit sahnenin uçan kamerası yalnız karenin kamerasına yazıyor
/// (`active_camera`), hedefli bir kamera hiçbir zaman o olmuyor.
const TOP_POS: Vec3 = Vec3::new(0.0, 7.0, 0.2);
const TOP_YAW: f32 = 0.0;
const TOP_PITCH: f32 = -1.15;

/// Dokunun PNG olarak yazıldığı kare.
const DUMP_FRAME: u32 = 60;

fn main() {
    let dump = std::env::var("GIZMO_RTT_DUMP").ok();

    App::<SimpleSceneState>::new("Gizmo Engine - Render To Texture", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);
            let torus = AssetManager::create_torus(device, 1.0, 0.35, 24, 16);

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(-1.8, 0.0, 0.0))
                    .with_rotation(Quat::from_rotation_x(0.5)),
                GlobalTransform::default(),
                torus,
                Material::new(white.clone()).with_pbr(Vec4::new(0.90, 0.50, 0.25, 1.0), 0.45, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(1.8, 0.0, 0.0)).with_scale(Vec3::splat(0.8)),
                GlobalTransform::default(),
                cube,
                Material::new(white.clone()).with_pbr(Vec4::new(0.30, 0.65, 0.95, 1.0), 0.45, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.4, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 24.0),
                Material::new(white).with_pbr(Vec4::new(0.14, 0.15, 0.17, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.6),
                intensity: 2.6,
                // `GIZMO_RTT_NO_SHADOW=1` güneşi gölgesiz bir dolgu ışığına çeviriyor (aşağıdaki
                // tarihteki ölçüm notu: gölge kaskadları kameradan TÜRETİLEN ayrı bir uniform ailesi).
                role: if std::env::var("GIZMO_RTT_NO_SHADOW").is_ok() {
                    gizmo::renderer::components::LightRole::Generic
                } else {
                    gizmo::renderer::components::LightRole::Sun
                },
                ..Default::default()
            });

            // İki kamera: biri önden (pencere, `primary`), biri tepeden (kendi dokusu).
            scene.spawn_camera(state, Vec3::new(0.0, 1.4, 6.5), Vec3::ZERO);
            let front = scene
                .world
                .query::<(Entity, &Camera)>().iter().next().map(|(entity, _)| entity);
            if let Some(id) = front.and_then(|id| scene.world.live(id)) {
                scene.world.add_component(id, CameraTag(0));
            }
            let mut top = Camera::new(55_f32.to_radians(), 0.1, 300.0, TOP_YAW, TOP_PITCH, false);
            top.exposure = 1.0;
            scene.world.spawn_bundle((
                Transform::new(TOP_POS),
                GlobalTransform::default(),
                top,
                CameraTag(1),
                // Kameranın dokusu: motor bu kamerayı her kare buraya çiziyor.
                CameraTarget::new(&scene.renderer.gpu_context(), OFF_W, OFF_H),
            ));

            scene.world.insert_resource(RttReport {
                size: (OFF_W, OFF_H),
                ..Default::default()
            });
        })
        .add_update_hook(|world, _state, _dt, _input| {
            if let Some(mut report) = world.get_resource_mut::<RttReport>() {
                report.frame += 1;
            }
        })
        .set_render_stage(RenderWorld::new().with_pass(dump_target(dump.clone())))
        .set_ui(move |world, _state, ctx| {
            let Some(r) = world.get_resource::<RttReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("rtt".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(440.0);
                        ui.heading("Dokuya çizim");
                        ui.label(format!("kare {}", r.frame));
                        ui.label(format!("tepedeki kameranın dokusu {}x{}", r.size.0, r.size.1));
                        ui.separator();
                        ui.label("her karede İKİ görüntü:");
                        ui.label("  1) tepeden kamera -> kendi dokusu (CameraTarget)");
                        ui.label("  2) önden kamera   -> pencere");
                        ui.separator();
                        ui.label("motor hedefli kamerayı karenin görüntüsünden önce,");
                        ui.label("kendi gönderiminde ve kendi geçmişiyle çiziyor.");
                        ui.label("oyun kodunda kamera seçimi ya da ikinci çizim yok.");
                        if let Some(dir) = &dump {
                            ui.separator();
                            ui.monospace(format!("{DUMP_FRAME}. karede -> {dir}/rtt_offscreen.png"));
                        }
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// `GIZMO_RTT_DUMP` verildiyse tepedeki kameranın dokusunu `DUMP_FRAME`. karede PNG olarak yazan
/// geçiş. Karenin üstüne çizilen bir geçiş olarak koşuyor (`with_pass`): o anda hedefli kameranın
/// dokusu çizilip gönderilmiş, ve kopyada kameranın `CameraTarget`'i duruyor.
fn dump_target(
    dir: Option<String>,
) -> impl FnMut(&World, &mut wgpu::CommandEncoder, &wgpu::TextureView, &mut Renderer) + Send + 'static {
    let mut frame = 0u32;
    move |copy, _encoder, _view, renderer| {
        frame += 1;
        let Some(dir) = dir.as_deref().filter(|_| frame == DUMP_FRAME) else { return };
        let Some(target) = copy.query::<&CameraTarget>().iter().next().cloned() else { return };
        let path = std::path::Path::new(dir).join("rtt_offscreen.png");
        let (w, h) = target.size();
        match gizmo::renderer::capture::texture_to_png(&renderer.device, &renderer.queue, target.texture(), &path) {
            Ok(()) => gizmo::gizmo_log!(Info, "tepedeki kameranın dokusu yazıldı: {} ({w}x{h})", path.display()),
            Err(e) => gizmo::gizmo_log!(Warning, "doku yazılamadı: {e}"),
        }
    }
}
