//! # Pozlama ve göz uyumu
//!
//! Karanlık bir koridordan güneşe çıkınca gözün yavaşça kısılması. Bir çizim motorunda bu,
//! karenin parlaklığını ölçüp pozlamayı ona göre kaydırmak: ölç → yumuşat → uygula.
//!
//! ## Motorda halkanın yarısı yerleşik — ama öteki yarısı **yazılabilir**
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | pozlamayı **uygulamak** | var — [`Camera::exposure`], doğrusal bir çarpan |
//! | kare parlaklığını **okumak** | var — `renderer.post.hdr_texture` genel, `COPY_SRC` taşıyor |
//! | yerleşik indirgeme geçişi / histogram | **yok** — ve asıl eksik olan bu |
//! | uyum hızı / yumuşatma sabiti | **yok** — oyun kendi yazıyor (bu demo yazıyor) |
//! | en düşük/en yüksek EV, hedef parlaklık | **yok** — aynı şekilde |
//! | ölçüm maskesi (merkez ağırlıklı, spot) | **yok** |
//! | fiziksel kamera (diyafram/enstantane/ISO) | **yok** — `exposure` ham bir çarpan, EV değil |
//!
//! Bu satır bir düzeltme. Daha önce burada "**aktüatör var, sensör yok**" yazıyordu ve
//! `docs/API_DEPTH.md` eksiği "bağlanabilir bir HDR hedefi" diye adlandırıyordu. Yanlıştı: HDR ara
//! hedefi `RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC` ile kuruluyor ve `hdr_texture`,
//! `hdr_texture_view`, `hdr_bind_group` üçü de genel. Bakılan yer yüzeydi (takas zinciri), ve o
//! gerçekten okunamıyor — ama post-process zincirinin okuduğu hedef okunabiliyor.
//!
//! Eksik olan **erişim değil, indirgeme**: ağaçtaki dokuz compute shader'ının hiçbiri indirgeme ya
//! da histogram değil, ve pozlama tek bir yerde, ACES eğrisinin hemen öncesinde, doğrusal olarak
//! uygulanıyor (`post_process.wgsl`).
//!
//! ## Ölçülen 0: halka kapanıyor, ve bedeli 10 ms
//!
//! Demo sensörü **yazıyor** (`sense_luminance`): HDR hedefini CPU'ya çekiyor, her 8. pikseli
//! örnekleyip ortalama doğrusal luma'yı çıkarıyor, sonra `pozlama = 0,18 / ölçüm` hedefine
//! `ADAPT` oranında yaklaşıyor. `GIZMO_AE_LOOP=1` ile açılıyor.
//!
//! Ölçüldü (2026-08-23, 400 kare, 12 karede bir örnek):
//!
//! | istasyon | ölçülen luma | yerleştiği pozlama | 0,18/luma |
//! |----------|--------------|--------------------|-----------|
//! | parlak | 0,3656 | **0,492** | 0,492 |
//! | karanlık | 0,0534 | **3,370** | 3,371 |
//!
//! Halka tam hedefe oturuyor. Ve karede de görünüyor — sağ yarı, HUD altı, kare 400:
//!
//! | istasyon | açık halka | kapalı halka | değişim |
//! |----------|------------|--------------|---------|
//! | parlak | 174,77 | **140,04** | −34,73 |
//! | karanlık | 56,59 | **119,11** | +62,52 |
//!
//! İki istasyonun arasındaki fark **118,18'den 20,93'e** düşüyor — yani göz uyumunun yapması
//! gereken şeyin **%82'si**. Parlak taraf kısılıyor, karanlık taraf açılıyor.
//!
//! ### Ölçülen 1: ve bu yüzden asıl eksik indirgeme
//!
//! Bir ölçümün maliyeti **10,1–10,9 ms** (ilk ölçüm 15–17 ms, ısınma). 12 karede bir yapıldığı
//! için kare başına ~0,87 ms — 60 Hz'lik bir karenin **%5'i**, tek bir sayı için.
//!
//! Maliyetin tamamı `map_async` + `poll(Wait)`: GPU bekletiliyor. Bir compute indirgeme aynı sayıyı
//! mikrosaniye mertebesinde verirdi ve GPU'yu hiç durdurmazdı. Yani "sensör yazılabilir" ile
//! "sensör kullanılabilir" arasındaki fark tam olarak bu 10 ms, ve motorun kapatması gereken boşluk
//! da bu — hedefe erişim değil.
//!
//! Bir de bir karelik gecikme var: sensör `default_render_pass`'ten sonra okuyor ama ana encoder
//! henüz gönderilmemiş, yani HDR hedefinde bir önceki karenin sonucu duruyor. Uyum zaten
//! yumuşatmalı olduğu için sonucu değiştirmiyor, ama saklanacak bir şey de değil.
//!
//! ### İki ölü kardeş alan
//!
//! `exposure` adında üç alan var ve ikisi ölü: `SceneUniforms::exposure` (yalnız yerleşim
//! kararlılığı için duruyor, shader'ın kendisi öyle diyor) ve `Renderer::exposure` (hiçbir şey
//! okumuyor — `docs/CAPABILITY_GAPS.md` §G bir demonun kaydırağının bu ölü alana bağlanmış
//! olduğunu kaydediyor). Canlı olan `Camera::exposure`.
//!
//! ## Ölçüldü — A: parlaklık savruluyor, hiçbir şey geri çekmiyor
//!
//! İki istasyon, pozlama ikisinde de 1,0. Ölçüm sol yarının HUD altı, ortalama parlaklık
//! (2026-08-23, 948×1028):
//!
//! | istasyon | kare 120 | kare 700 | 580 karelik sürüklenme |
//! |----------|----------|----------|------------------------|
//! | parlak | 148,974 | 149,102 | **+0,128** |
//! | karanlık | 56,384 | 56,400 | **+0,015** |
//!
//! İki istasyon arasında **2,64 kat** fark var, ve on saniyeye yakın bir sürede hiçbiri
//! kımıldamıyor: sürüklenme yakalama gürültüsünün mertebesinde. Göz uyumu olan bir boru hattında
//! bu iki sayı bir iki saniye içinde birbirine yaklaşırdı. Halka **açık**.
//!
//! ## Ölçüldü — B: aktüatörün yetkisi var
//!
//! Aynı karanlık istasyon, yalnız `Camera::exposure` değişiyor:
//!
//! | pozlama | ortalama parlaklık | 0,5'e oran |
//! |---------|--------------------|------------|
//! | 0,5 | 31,847 | 1,000 |
//! | 1,0 | 56,384 | 1,771 |
//! | 2,0 | 90,912 | 2,855 |
//! | 4,0 | 130,401 | **4,095** |
//!
//! Yani sekiz kat pozlama, ekranda 4,1 kat parlaklık — ve bu, iki istasyon arasındaki 2,64 katı
//! **fazlasıyla** kapatmaya yeter. Karanlık istasyonu parlak istasyonun seviyesine çıkarmak için
//! gereken pozlama 5 civarında, yani menzilin içinde.
//!
//! Demek ki eksik olan yetki değil, **bilgi**: bir oyun pozlamayı istediği yere koyabiliyor, ama
//! nereye koyacağını yerleşik olarak söyleyen bir ölçüm yok. Yukarıdaki "Ölçülen 0" o bilgiyi
//! elle üretiyor ve halkayı kapatıyor — kare başına 0,87 ms'ye.
//!
//! Merdivenin doğrusal olmaması da bilgi: 0,5 → 1,0 katsayısı 1,77, 1,0 → 2,0'de 1,61, 2,0 → 4,0'da
//! 1,43'e düşüyor. Bu, ACES eğrisinin yukarıda yatmasıdır — pozlama eğrinin **öncesinde**,
//! doğrusal olarak uygulanıyor, ve eğri artışı yutuyor.
//!
//! ## Halkayı oyun tarafında kapatmanın bedeli
//!
//! Sensörü elle yazmak mümkün ama pahalı: ağaçtaki tek geri okuma yolu
//! `gizmo_renderer::capture::texture_to_png`, ve kendi belgesi onu "bir tanılama" diye anıyor —
//! kopyalama komutunu kendi gönderiyor, okuma dönene kadar **bloke oluyor**, ve üstüne bir de
//! PNG kodluyor. Kare başına çalıştırılacak bir şey değil.
//!
//! ## Kontroller
//!   * `GIZMO_AE_LOOP=1` — halkayı kapat (sensör + uyum)
//!   * `GIZMO_AE_SELFTEST=1` — her ölçümü konsola yaz
//!   * `GIZMO_AE_POS=parlak|karanlik` — kamerayı iki istasyondan birine koy
//!   * `GIZMO_AE_EXPOSURE=<sayı>` — elle pozlama (varsayılan 1,0)
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// İki istasyon: aydınlık dış alan ve karanlık iç mekân.
const BRIGHT_POS: Vec3 = Vec3::new(0.0, 2.0, 14.0);
const DARK_POS: Vec3 = Vec3::new(0.0, 2.0, -13.0);

#[derive(Clone, Copy)]
struct Ae {
    exposure: f32,
    dark: bool,
    frame: u32,
    /// Halka kapalı mı — `GIZMO_AE_LOOP=1`.
    closed_loop: bool,
    /// Sensörün en son okuduğu sahne parlaklığı (doğrusal HDR, ortalama luma).
    measured: f32,
    /// Kaç kez ölçüldü.
    samples: u32,
    /// Bir ölçümün maliyeti, milisaniye.
    last_cost_ms: f32,
}
gizmo::core::impl_component!(Ae);

/// Sensörün kaç karede bir okuduğu. Okuma GPU'yu bekletiyor, o yüzden her kare değil — maliyeti
/// aşağıda ölçülü.
const SENSE_EVERY: u32 = 12;

/// Uyum hızı: ölçülen ile hedef arasındaki farkın kare başına kapanan oranı.
const ADAPT: f32 = 0.08;

/// Hedef ortalama parlaklık, doğrusal uzayda. Gözün "doğru pozlanmış" saydığı orta gri.
const TARGET: f32 = 0.18;

/// Yarım-duyarlıklı bir kayan noktayı çözer.
///
/// Elle, çünkü `half` demonun bağımlılığı değil ve bir okuma döngüsü için crate eklemek gereksiz.
/// `f16` de kararlı Rust'ta yok.
fn f16_to_f32(bits: u16) -> f32 {
    let sign = f32::from_bits(u32::from(bits & 0x8000) << 16);
    let exp = (bits >> 10) & 0x1f;
    let frac = bits & 0x03ff;
    let mag = match exp {
        // Sıfır ve normal-altı: 2^-24 adımlarla.
        0 => f32::from(frac) * 5.960_464_5e-8,
        // Sonsuz ve NaN. Kare parlaklığı için sonsuzu 0 saymak, ortalamayı NaN yapmaktan iyi.
        0x1f => return sign * if frac == 0 { f32::INFINITY } else { f32::NAN },
        _ => {
            // f16 üs sapması 15, f32'ninki 127.
            let e = u32::from(exp) + (127 - 15);
            f32::from_bits((e << 23) | (u32::from(frac) << 13))
        }
    };
    if bits & 0x8000 != 0 {
        -mag
    } else {
        mag
    }
}

/// **Sensör.** HDR ara hedefini okuyup sahnenin ortalama doğrusal parlaklığını döndürür.
///
/// Motorun bir indirgeme geçişi ya da histogramı yok — ama `renderer.post.hdr_texture` genel ve
/// `COPY_SRC` taşıyor, yani ölçüm *yazılabilir*. Bu, o yolun en kısa hâli: dokuyu CPU'ya çek,
/// seyrek bir ızgarayı örnekle, ortalamasını al.
///
/// Ucuz değil ve öyle olduğunu iddia etmiyor: `map_async` + `poll(Wait)` GPU'yu bekletiyor.
/// Maliyeti ölçülüyor ve arayüzde yazıyor. Bir compute indirgeme bunun binde biri olurdu, ve
/// motorda o yok — asıl eksik olan bu, hedefe erişim değil.
fn sense_luminance(renderer: &gizmo::renderer::Renderer) -> Option<(f32, f32)> {
    let t0 = std::time::Instant::now();
    let tex = &renderer.post.hdr_texture;
    let (w, h) = (tex.width(), tex.height());
    // Rgba16Float: kanal başına 2 bayt, piksel başına 8.
    const BPP: u32 = 8;
    let padded = (w * BPP).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

    let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ae_sensor"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = renderer
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ae_sensor"),
        });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue.submit(std::iter::once(enc.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |v| {
        let _ = tx.send(v);
    });
    let _ = renderer.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    rx.recv().ok()?.ok()?;
    let data = slice.get_mapped_range().ok()?;

    // Her 8. pikseli örnekle: ortalama parlaklık için fazlasıyla yeterli, ve okumanın kendisi
    // zaten baskın maliyet.
    let mut sum = 0.0f64;
    let mut n = 0u32;
    for y in (0..h).step_by(8) {
        let row = (y * padded) as usize;
        for x in (0..w).step_by(8) {
            let i = row + (x * BPP) as usize;
            let r = f16_to_f32(u16::from_le_bytes([data[i], data[i + 1]]));
            let g = f16_to_f32(u16::from_le_bytes([data[i + 2], data[i + 3]]));
            let b = f16_to_f32(u16::from_le_bytes([data[i + 4], data[i + 5]]));
            if !(r + g + b).is_finite() {
                continue;
            }
            sum += f64::from(0.2126 * r + 0.7152 * g + 0.0722 * b);
            n += 1;
        }
    }
    drop(data);
    staging.unmap();
    if n == 0 {
        return None;
    }
    Some((
        (sum / f64::from(n)) as f32,
        t0.elapsed().as_secs_f32() * 1000.0,
    ))
}

fn config() -> (f32, bool) {
    let e = std::env::var("GIZMO_AE_EXPOSURE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0f32)
        .clamp(0.05, 16.0);
    (e, matches!(std::env::var("GIZMO_AE_POS").as_deref(), Ok("karanlik")))
}

fn main() {
    let (exposure, dark) = config();

    App::<SimpleSceneState>::new("Gizmo Engine - Auto Exposure", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Aydınlık taraf: açık zemin, güneş altında.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, 10.0)).with_scale(Vec3::new(20.0, 0.4, 20.0)),
                GlobalTransform::default(),
                cube.clone(),
                Material::new(white.clone()).with_pbr(Vec4::new(0.85, 0.83, 0.78, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            // Karanlık taraf: koyu zemin ve üstünü kapatan bir tavan.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, -10.0)).with_scale(Vec3::new(20.0, 0.4, 20.0)),
                GlobalTransform::default(),
                cube.clone(),
                Material::new(white.clone()).with_pbr(Vec4::new(0.10, 0.10, 0.12, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 6.0, -10.0)).with_scale(Vec3::new(20.0, 0.4, 22.0)),
                GlobalTransform::default(),
                cube.clone(),
                Material::new(white.clone()).with_pbr(Vec4::new(0.08, 0.08, 0.09, 1.0), 0.95, 0.0),
                MeshRenderer::new(),
            ));

            // İki tarafta da aynı nesneler — karşılaştırılacak şey aydınlatma, geometri değil.
            for side in [10.0f32, -10.0] {
                for i in 0..5 {
                    scene.world.spawn_bundle((
                        Transform::new(Vec3::new((i as f32 - 2.0) * 2.2, 1.1, side))
                            .with_scale(Vec3::splat(0.8)),
                        GlobalTransform::default(),
                        cube.clone(),
                        Material::new(white.clone()).with_pbr(
                            Vec4::new(0.70, 0.45, 0.30, 1.0),
                            0.5,
                            0.0,
                        ),
                        MeshRenderer::new(),
                    ));
                }
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.2) * Quat::from_rotation_x(-0.9),
                intensity: 3.4,
                ..Default::default()
            });
            let _ = white;

            let pos = if dark { DARK_POS } else { BRIGHT_POS };
            let look = if dark {
                Vec3::new(0.0, 1.0, -10.0)
            } else {
                Vec3::new(0.0, 1.0, 10.0)
            };
            scene.spawn_camera(state, pos, look);
            scene.world.insert_resource(Ae {
                exposure,
                dark,
                frame: 0,
                closed_loop: std::env::var("GIZMO_AE_LOOP").is_ok(),
                measured: 0.0,
                samples: 0,
                last_cost_ms: 0.0,
            });
            gizmo::gizmo_log!(
                Info,
                "istasyon: {} · elle pozlama: {}",
                if dark { "karanlık" } else { "parlak" },
                exposure
            );
        })
        // Pozlama her karede yazılıyor: aktüatörün canlı olduğunu göstermenin yolu bu.
        .add_update_system(apply_exposure.in_phase(Phase::Update))
        .set_render(|world, _state, encoder, view, renderer, _lt| {
            gizmo::systems::default_render_pass(world, encoder, view, renderer);

            let Some(mut ae) = world.get_resource::<Ae>().map(|a| *a) else {
                return;
            };
            if !ae.closed_loop || !ae.frame.is_multiple_of(SENSE_EVERY) {
                return;
            }

            // Sensör, çizimden SONRA — ama `encoder` henüz gönderilmedi, yani HDR hedefinde
            // duran bir ÖNCEKİ karenin sonucu. Bir karelik gecikme; uyum zaten yumuşatmalı
            // olduğu için bir sorun değil, ama gizlenecek bir şey de değil.
            if let Some((lum, cost)) = sense_luminance(renderer) {
                ae.measured = lum;
                ae.samples += 1;
                ae.last_cost_ms = cost;

                // Ölç -> yumuşat -> uygula. Hedefe götürecek pozlama TARGET/ölçüm; oraya
                // ADAPT oranında yaklaşılıyor.
                if lum > 1e-5 {
                    let want = (TARGET / lum).clamp(0.05, 16.0);
                    ae.exposure += (want - ae.exposure) * ADAPT * SENSE_EVERY as f32;
                    ae.exposure = ae.exposure.clamp(0.05, 16.0);
                }

                if std::env::var("GIZMO_AE_SELFTEST").is_ok() {
                    gizmo::gizmo_log!(
                        Info,
                        "kare {:>4} · ölçülen luma {:.4} · pozlama {:.3} · ölçüm {:.2} ms",
                        ae.frame,
                        ae.measured,
                        ae.exposure,
                        ae.last_cost_ms
                    );
                }
                world.insert_resource(ae);
            }
        })
        .set_ui(|world, _state, ctx| {
            let Some(a) = world.get_resource::<Ae>().map(|a| *a) else {
                return;
            };
            gizmo::egui::Area::new("ae".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(410.0);
                        ui.heading("Pozlama");
                        ui.label(format!(
                            "istasyon: {} · pozlama {:.2} · kare {}",
                            if a.dark { "karanlık" } else { "parlak" },
                            a.exposure,
                            a.frame
                        ));
                        ui.separator();
                        ui.label("Camera::exposure yazılabiliyor — aktüatör CANLI.");
                        if a.closed_loop {
                            ui.colored_label(
                                gizmo::egui::Color32::from_rgb(120, 200, 130),
                                "halka KAPALI — sensör bu demonun kendi kodu",
                            );
                            ui.monospace(format!(
                                "  ölçülen luma {:.4} · hedef {TARGET:.2}",
                                a.measured
                            ));
                            ui.monospace(format!(
                                "  {} ölçüm · sonuncusu {:.2} ms",
                                a.samples, a.last_cost_ms
                            ));
                            ui.label("10 ms'nin tamamı map_async+poll: GPU bekliyor.");
                        } else {
                            ui.colored_label(
                                gizmo::egui::Color32::from_rgb(230, 160, 80),
                                "halka AÇIK — GIZMO_AE_LOOP=1 ile kapanır",
                            );
                        }
                        ui.label("hdr_texture okunabiliyor; eksik olan indirgeme geçişi.");
                        ui.separator();
                        ui.label("SceneUniforms::exposure ve Renderer::exposure ÖLÜ;");
                        ui.label("canlı olan Camera::exposure.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Pozlamayı kameraya yazar. Motorda bunu **otomatik** yapan bir şey yok; değeri veren biziz.
fn apply_exposure(mut cameras: Query<Mut<Camera>>, mut ae: ResMut<Ae>) {
    ae.frame += 1;
    let e = ae.exposure;
    for (_entity, mut camera) in cameras.iter_mut() {
        camera.exposure = e;
    }
}
