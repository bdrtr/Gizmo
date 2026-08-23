//! # Hacimsel sis ve tanrı ışınları
//!
//! Işığın havadaki toza çarpıp **görünür** hâle gelmesi: pencereden düşen huzmeler, ormanda
//! ağaçların arasından süzülen ışık. Yüzeye değil, aradaki hacme uygulanan bir etki.
//!
//! ## Motorda var — ve **2026-08-23'ten beri altı ayarı da var**
//!
//! Tanrı ışınları çalışıyor: [`VolumetricState`] bir ışın yürütme geçişi kuruyor ve kareye
//! uyguluyor. Eskiden `VolumetricState`'in genel yüzeyi yalnız GPU nesneleriydi — doku, görünüm,
//! boru hattı, bağlama grubu, genişlik, yükseklik — ve etkiyi belirleyen her sayı
//! gölgelendiricide gömülüydü:
//!
//! | ne | gölgelendiricideki değer | şimdi |
//! |----|--------------------------|-------|
//! | faz anizotropisi (g) | `0.55` | `VolumetricParams::phase_g` |
//! | ışın yürütme adımı | `16` | `::steps` |
//! | yürütme mesafesi tavanı | `100.0` m | `::max_distance` |
//! | güneş saçılım katsayısı | `0.0015` | `::sun_scatter` |
//! | ampul saçılım katsayısı | `0.0008` | `::bulb_scatter` |
//! | gölge sapması | `0.16` | `::shadow_bias` |
//!
//! Altısı 32 baytlık tek bir uniform'da ve her kare yükleniyor: `vol.params.steps = 8.0` demek
//! yeterli, yeniden kurulacak bir şey yok. Varsayılanlar gölgelendiricide duran değerlerin ta
//! kendisi — `the_defaults_are_the_shader_literals_they_replaced` bunu piksel piksel kilitliyor,
//! yani sabitleri dışarı çıkarmak hiçbir sahnenin görünüşünü değiştirmedi.
//!
//! ## Kapatmak da artık yok etmek değil
//!
//! `VolumetricState::enabled` var (2026-08-23). Eskiden tek kapatma yolu `renderer.volumetric =
//! None`'du ve bu **durumu yok ediyordu**: geri açmak `VolumetricState::new(device, scene,
//! deferred, w, h)` ile yeniden kurmak demekti. Aynı bayrak SSR, SSGI, SSAO ve dekallerde de var
//! (`docs/CAPABILITY_GAPS.md` §B).
//!
//! ## Mesafe sisi de sabit, ve tabanı kıpırdamıyor
//!
//! Mesafe sisi dört gömülü preset + bir karışım katsayısı. Renk, yoğunluk ve yükseklik düşüşü
//! gölgelendiricide sabit — "yoğunluk 0,02" cümlesi kurulamıyor.
//!
//! Ve sis düzleminin yüksekliği preset'e göre bile değişmiyor: `deferred_lighting.wgsl:254`'te
//! tek bir satır, `fog_base_height = -5.0`. Sis tabakası yukarı ya da aşağı taşınamıyor.
//!
//! ## Sürüklenen / gürültü sürücülü sis yok
//!
//! Motorun hiçbir yerinde `yoğunluk = fbm(p − rüzgâr·t)` yok, ve yazılacak bir rüzgâr vektörü de
//! yok: ne uniform, ne bileşen, ne `Material` alanı. Zamanla değişen tek hacimsel yoğunluk
//! `SmokeVolume`'un taşınan ızgarası, ve orada bile gürültü **hıza** uygulanıyor — yoğunluk
//! kaynak enjeksiyonu + yarı-Lagrange taşıma + dağılmadan geliyor, yani duran ve sürüklenen bir
//! sis olarak yazılamıyor.
//!
//! ## Ölçüldü — etki gerçek, ayar sıfır
//!
//! Aynı sahne iki kez, tek fark hacimsel geçişin yok edilip edilmemesi (2026-08-23, 948×1028,
//! sol yarı, HUD altı):
//!
//! | | değer |
//! |---|-------|
//! | farklı piksel | **%19,55** |
//! | ortalama fark | 12,01 |
//! | en büyük kanal farkı | **134** |
//! | fark kutusu | tüm kare |
//! | ortalama parlaklık, yok | 117,63 |
//! | ortalama parlaklık, açık | 114,87 (**−2,76**) |
//!
//! Etki karenin beşte birine dokunuyor ve yer yer 134 seviyelik fark yapıyor — yani çalışıyor,
//! ve zayıf da değil. Parlaklığın **düşmesi** de doğru işaret: hacimsel saçılım yalnız ışık
//! eklemiyor, ışığın önünü de kesiyor.
//!
//! ## Ölçülen 2: altı ayarın beşi bu sahnede kareyi değiştiriyor
//!
//! `GIZMO_VOL_KNOB=<0..6>` ayarı tek bir adıma kilitliyor; her adım varsayılandan başlıyor, yani
//! tek değişken adı yazan alan. Aynı kare (240), sol yarı, 948×1028 (2026-08-23):
//!
//! | ayar | en büyük kanal farkı | ortalama parlaklık |
//! |------|----------------------|--------------------|
//! | varsayılan (referans) | 0 | 120,26 |
//! | `g = 0,0` izotropik | **12** | 121,11 (+0,85) |
//! | `g = 0,9` ileri saçılım | **8** | 120,07 (−0,19) |
//! | `adım = 4` | **4** | 120,23 |
//! | `tavan = 20 m` | **4** | 120,25 |
//! | `güneş ×4` | **10** | 120,90 (+0,64) |
//! | `ampul ×60` | 0 | 120,26 |
//!
//! Faz iki yönde de doğru davranıyor: izotropik yapmak huzmeyi yayıyor ve kareyi aydınlatıyor,
//! ileri saçılıma itmek toparlayıp karartıyor.
//!
//! `ampul` sıfır çünkü **bu sahnede nokta ışık yok** — ölçtüğü döngü yalnız nokta ve spot ışıklar
//! için koşuyor. Bağlı olduğu ayrı ölçülü: motorun `every_volumetric_parameter_changes_the_frame`
//! testi ışının içine bir lamba koyup 3590/16384 piksel oynattığını gösteriyor.
//!
//! `adım`ın 4'te kalması da doğru davranış, kusur değil: saçılım `Σ katkı × adım_boyu` olarak
//! toplanıyor ve `adım_boyu = tavan / adım`, yani katkının sabit olduğu bir aralıkta toplam adım
//! sayısından **bağımsız**. Adım sayısı değeri değil *doğruluğu* değiştiriyor — motorun
//! `the_step_count_refines_the_march_without_changing_what_it_renders` testi tam bu şekli
//! ölçüyor: 4 ile 64 adım 2755 pikselde farklı, ama en büyük kanal farkı 6.
//!
//! ### Ölçüm notu: turu ölçmek gürültüyü ölçmekti
//!
//! İlk ölçüm turu koşarken alındı — her 60 karede bir ayar değişiyor, ve kare 30'un varsayılanı
//! kare 90'ın izotropiğiyle karşılaştırılıyordu. Sayılar makul görünüyordu (%0,66 farklı piksel,
//! 77 kanal farkı) ama hepsi çöptü: **aynı ayarın** kare 30'u ile kare 45'i arasında %1,09 fark
//! ve **96** kanal farkı var. Sahnenin ışığı zamanla dönüyor, yani iki farklı karenin farkı
//! ayarın değil zamanın farkı, ve gürültü tabanı ölçmeye çalıştığım etkiden büyüktü.
//!
//! `GIZMO_VOL_KNOB` bunun için var: ayar sabitleniyor, kare sabitleniyor, geriye tek değişken
//! kalıyor. O kurulumda aynı ayarın iki ayrı koşusu **birebir aynı** kareyi veriyor — gürültü
//! tabanı 0 — ve 4 seviyelik bir fark bile gerçek.
//!
//! Dersi: **bir ölçümün gürültü tabanını ölçmeden sayısına güvenilmez.** Yukarıdaki 12 ve 10,
//! tabanı 96 olan bir kurulumda görünmezdi.
//!
//! ## Kontroller
//!   * `GIZMO_VOL=0` — hacimsel geçişi kapat (artık `enabled` bayrağıyla, yok etmeden)
//!   * `GIZMO_VOL_KNOB=<0..6>` — ayar turunu tek adıma kilitle (ölçüm için gerekli)
//!   * `GIZMO_VOL_SELFTEST=1` — tur her adım değiştirdiğinde konsola yaz
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Bir ayarın canlı değiştirilmesi için tutulan defter. `VolumetricParams`'ın kendisini tutmak
/// yerine bir kopya, çünkü `set_render` kapanışı `Renderer`'a ancak orada erişiyor.
#[derive(Clone, Copy)]
struct FogKnobs {
    params: gizmo::renderer::volumetric::VolumetricParams,
    /// Kaç karede bir gezilecek. Ölçüm koşusu bunun sayesinde tek koşuda altı ayarı da geziyor.
    frame: u32,
}
gizmo::core::impl_component!(FogKnobs);

/// Ayar turu: her adım bir alanı varsayılanından uzağa itiyor. Ölçüm bu turu geziyor.
const SWEEP: [(&str, fn(&mut gizmo::renderer::volumetric::VolumetricParams)); 7] = [
    ("varsayılan", |_| {}),
    ("g = 0,0 (izotropik)", |p| p.phase_g = 0.0),
    ("g = 0,9 (ileri saçılım)", |p| p.phase_g = 0.9),
    ("adım = 4", |p| p.steps = 4.0),
    ("tavan = 20 m", |p| p.max_distance = 20.0),
    ("güneş ×4", |p| p.sun_scatter = 0.006),
    ("ampul ×60", |p| p.bulb_scatter = 0.048),
];

/// Her ayarın kaç kare gösterileceği.
const SWEEP_EVERY: u32 = 60;

fn main() {
    let on = !matches!(std::env::var("GIZMO_VOL").as_deref(), Ok("0"));

    App::<SimpleSceneState>::new("Gizmo Engine - Volumetric Fog", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Işığı kesen sütunlar: huzmelerin görünmesi için gölge gerekiyor.
            for i in 0..5 {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new((i as f32 - 2.0) * 2.4, 2.2, -2.0))
                        .with_scale(Vec3::new(0.5, 4.4, 0.5)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.30, 0.31, 0.34, 1.0),
                        0.8,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 40.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.40, 0.40, 0.43, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));

            // Güneş alçaktan ve sütunların arkasından: huzmeler kameraya doğru düşsün.
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.05) * Quat::from_rotation_x(-0.28),
                intensity: 4.0,
                ..Default::default()
            });
            let _ = white;
            scene.spawn_camera(state, Vec3::new(0.0, 1.0, 7.0), Vec3::new(0.0, 1.2, -2.0));
        })
        .set_render(move |world, _state, encoder, view, renderer, _lt| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            // Kapatmak artık durumu yok etmiyor: bir bayrak.
            if let Some(vol) = renderer.volumetric.as_mut() {
                vol.enabled = on;

                let mut knobs = world
                    .get_resource::<FogKnobs>()
                    .map(|k| *k)
                    .unwrap_or(FogKnobs {
                        params: Default::default(),
                        frame: 0,
                    });
                knobs.frame += 1;

                // Ayarı her SWEEP_EVERY karede bir değiştir. Her adım varsayılandan başlıyor,
                // yani turdaki tek değişken adı yazan alan.
                //
                // `GIZMO_VOL_KNOB=<n>` turu durdurup tek adıma kilitliyor. Ölçüm bunu kullanmak
                // ZORUNDA: sahnenin ışığı zamanla dönüyor, yani iki farklı karenin farkı ayarın
                // değil zamanın farkı. Ölçüm notuna bakın.
                let step = match std::env::var("GIZMO_VOL_KNOB").ok().and_then(|v| v.parse::<usize>().ok())
                {
                    Some(n) => n.min(SWEEP.len() - 1),
                    None => (knobs.frame / SWEEP_EVERY) as usize % SWEEP.len(),
                };
                let mut p = gizmo::renderer::volumetric::VolumetricParams::default();
                SWEEP[step].1(&mut p);
                knobs.params = p;
                vol.params = p;

                if std::env::var("GIZMO_VOL_SELFTEST").is_ok()
                    && knobs.frame.is_multiple_of(SWEEP_EVERY)
                {
                    gizmo::gizmo_log!(
                        Info,
                        "kare {:>4} · ayar {:24} · g {:.2} adım {:.0} tavan {:.0} güneş {:.4} \
                         ampul {:.4} sapma {:.2}",
                        knobs.frame,
                        SWEEP[step].0,
                        p.phase_g,
                        p.steps,
                        p.max_distance,
                        p.sun_scatter,
                        p.bulb_scatter,
                        p.shadow_bias
                    );
                }
                world.insert_resource(knobs);
            }

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(move |world, _state, ctx| {
            let knobs = world.get_resource::<FogKnobs>().map(|k| *k);
            gizmo::egui::Area::new("vf".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(420.0);
                        ui.heading("Hacimsel sis");
                        ui.label(format!("hacimsel geçiş: {}", if on { "açık" } else { "yok edildi" }));
                        ui.separator();
                        if let Some(k) = knobs {
                            let step = (k.frame / SWEEP_EVERY) as usize % SWEEP.len();
                            ui.label(format!("ayar turu: {}", SWEEP[step].0));
                            ui.monospace(format!(
                                "  g = {:.2} · adım = {:.0} · tavan = {:.0} m",
                                k.params.phase_g, k.params.steps, k.params.max_distance
                            ));
                            ui.monospace(format!(
                                "  güneş {:.4} · ampul {:.4} · sapma {:.2}",
                                k.params.sun_scatter, k.params.bulb_scatter, k.params.shadow_bias
                            ));
                            ui.label("altısı da VolumetricParams alanı — canlı yazılıyor.");
                        }
                        ui.separator();
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(120, 200, 130),
                            "enabled bayrağı var — kapatmak yok etmek değil",
                        );
                        ui.separator();
                        ui.label("mesafe sisi: 4 preset + karışım katsayısı.");
                        ui.label("fog_base_height = -5.0, preset'e göre bile değişmiyor.");
                        ui.label("sürüklenen/gürültülü sis: hiç yok.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
