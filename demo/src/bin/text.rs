//! # Metin çizimi
//!
//! Motor 2026-08-24'e kadar tek bir glif çizemiyordu. Etiket isteyen her demo bir `egui` paneli
//! açıyordu — hepsinin hata ayıklama kaplaması gibi görünmesinin sebebi buydu
//! (`docs/CAPABILITY_GAPS.md` §A2, §E'nin ilk sıraya koyduğu boşluk). Bu demo o boşluğun
//! kapanan kısmını gösteriyor: `Text` bileşeni, iki uzay, dokuz çapa ve derinlik testi.
//!
//! ## Font motorla gelmiyor, ve bu bilerek
//!
//! Bir yazı tipi **lisans** kararıdır ve karar depoyu tutanın; render katmanının değil. Bu yüzden
//! ne varsayılan font var ne de yedeğe düşme: fontu yüklenmemiş bir `Text` sessizce başka bir
//! yazı tipine geçmez, **hiçbir şey çizmez**. Demo fontu şu sırayla arıyor:
//!
//!   1. `GIZMO_FONT` ortam değişkeni,
//!   2. birkaç bilinen sistem yolu (`/usr/share/fonts/...`),
//!   3. hiçbiri yoksa motorun kendi **sentetik** yüzü — üç glifli, biri dolu bir kutu.
//!
//! Üçüncü hâlde ekranda yazı değil kutular görürsünüz; bu bir hata değil, "font bulunamadı"nın
//! görünür hâli. Gerçek metin için: `GIZMO_FONT=/yol/font.ttf cargo run --release -p demo --bin text`
//!
//! ## Ne var
//!
//! | yetenek | durum |
//! |---------|-------|
//! | ekran uzayı (pencere pikseli, sol-üst orijin) | var |
//! | dünya uzayı (kameraya dönük dörtgen) | var, **derinlik testli** |
//! | dokuz çapa | var |
//! | `\n` ile satır | var |
//! | kerning | fontun `kern` tablosundan |
//! | sarma, şekillendirme (shaping), iki yönlü metin, font yedeklemesi | **yok** |
//!
//! Ölçülen: aynı sahne metinli ve metinsiz 16 384 pikselin **869**'unda farklı; sol-üste
//! yerleştirilen yazı sol-üst çeyrekte 869, sağ-alt çeyrekte **0** piksel değiştiriyor; ve önüne
//! duvar konan bir dünya etiketi **tam 0** piksel değiştiriyor (derinlik testi çıkarılınca 1 589).
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Font aranacak bilinen yollar. Bulunan ilki kullanılır.
const FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    "C:/Windows/Fonts/arial.ttf",
];

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Text", 1280, 720)
        .with_simple_scene(|scene, state| {
            let (font, real) = load_font(scene.renderer);

            // ── Zemin ve bir duvar ───────────────────────────────────────────────────────
            // Duvarın tek işi: arkasındaki dünya etiketinin gerçekten gizlendiğini göstermek.
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -2.0, 0.0)).with_scale(Vec3::new(30.0, 0.2, 30.0)),
                GlobalTransform::default(),
                AssetManager::create_cube(&scene.renderer.device),
                Material::new(white.clone()).with_pbr(Vec4::new(0.22, 0.23, 0.26, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(3.0, 0.0, 2.0)).with_scale(Vec3::new(2.0, 2.0, 0.2)),
                GlobalTransform::default(),
                AssetManager::create_cube(&scene.renderer.device),
                Material::new(white.clone()).with_pbr(Vec4::new(0.75, 0.32, 0.30, 1.0), 0.6, 0.0),
                MeshRenderer::new(),
            ));

            // ── Dünya etiketleri ─────────────────────────────────────────────────────────
            // Soldaki açıkta, sağdaki duvarın TAM ARKASINDA: kamera ileri gidince biri kaybolur,
            // diğeri kalır. Derinlik testinin gözle görülür hâli bu.
            for (x, z, label, color) in [
                (-3.0_f32, 0.0_f32, "ACIKTA", Vec4::new(0.6, 1.0, 0.7, 1.0)),
                (3.0, 0.0, "DUVAR ARKASI", Vec4::new(1.0, 0.6, 0.6, 1.0)),
            ] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.5, z)),
                    GlobalTransform::default(),
                    // 48 px'lik glifler, birim başına 48 px → bir birim yüksekliğinde yazı.
                    // `size_px`'i büyütmek yazıyı büyütmez, KESKİNLEŞTİRİR: ölçek px_per_unit'te.
                    Text::world(label, font, 48.0, 48.0).with_color(color),
                ));
            }

            // ── Ekran metni: sekiz çapa, kendi kenarında ─────────────────────────────────
            //
            // Konumlar burada değil `reposition`'da hesaplanıyor: `TextSpace::Screen` mutlak
            // pencere pikseli tutuyor, yani pencere boyutunu bilen tek yer güncelleme. Kurulumda
            // sabitleseydik demo yalnız açılış boyutunda doğru olurdu — ve tam olarak çapaları
            // gösteren bir demonun yanlış olabileceği en kötü yer bu.
            let mut layout = ScreenLayout::default();
            for (anchor, fx, fy, label) in [
                (TextAnchor::TopLeft, 0.0, 0.0, "sol-ust"),
                (TextAnchor::TopCenter, 0.5, 0.0, "ust-orta"),
                (TextAnchor::TopRight, 1.0, 0.0, "sag-ust"),
                (TextAnchor::CenterLeft, 0.0, 0.5, "sol-orta"),
                (TextAnchor::CenterRight, 1.0, 0.5, "sag-orta"),
                (TextAnchor::BottomLeft, 0.0, 1.0, "sol-alt"),
                (TextAnchor::BottomCenter, 0.5, 1.0, "alt-orta"),
                (TextAnchor::BottomRight, 1.0, 1.0, "sag-alt"),
            ] {
                let e = scene.world.spawn_bundle((Text::screen(label, font, 22.0, Vec2::ZERO)
                    .with_anchor(anchor),));
                layout.0.push((e.id(), Vec2::new(fx, fy), MARGIN));
            }

            // Ortada: çok satırlı, ve fontun gerçek olup olmadığını söyleyen satır.
            let note = if real {
                "Gizmo metin cizimi\niki uzay . dokuz capa . derinlik testi\nGIZMO_FONT ile font degistirilebilir"
            } else {
                "FONT BULUNAMADI\nsentetik yuz cizliyor (kutular)\nGIZMO_FONT=/yol/font.ttf ile calistirin"
            };
            let e = scene.world.spawn_bundle((Text::screen(note, font, 26.0, Vec2::ZERO)
                .with_anchor(TextAnchor::TopCenter)
                .with_color(Vec4::new(1.0, 0.95, 0.7, 1.0)),));
            layout.0.push((e.id(), Vec2::new(0.5, 0.72), 0.0));

            // ── Boyut taraması ───────────────────────────────────────────────────────────
            // Aynı dize, artan `size_px`. Atlas HER BOYUTU ayrı raster tutuyor (bkz. `GlyphKey`),
            // yani bu döngü atlasa beş ayrı kopya koyuyor — ve bir boyut kaydırıcısının neden tam
            // piksele yuvarlandığını gösteren şey de bu: yuvarlamasaydı her kare yeni kopyalar.
            let mut y = 0.20_f32;
            for size in [10.0_f32, 14.0, 20.0, 28.0, 40.0] {
                let e = scene.world.spawn_bundle((Text::screen(
                    format!("{size:.0} px"),
                    font,
                    size,
                    Vec2::ZERO,
                )
                .with_anchor(TextAnchor::TopRight),));
                layout.0.push((e.id(), Vec2::new(1.0, y), MARGIN));
                y += 0.055;
            }
            scene.world.insert_resource(layout);

            scene.spawn_camera(state, Vec3::new(0.0, 1.0, 9.0), Vec3::new(0.0, 0.5, 0.0));
        })
        // `with_simple_scene` kendi güncellemesini (uçan kamera) kurar ve `set_update` onu
        // DEĞİŞTİRİR — bu yüzden kamerayı elle çağırıyoruz. `simple_scene_update` tam olarak
        // bunun için `pub`.
        .set_update(|world, state, dt, input| {
            gizmo::simple::simple_scene_update(world, state, dt, input);
            reposition(world);
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Ekran kenarlarından bırakılan boşluk, piksel.
const MARGIN: f32 = 12.0;

/// Hangi ekran metni pencerenin neresine oturuyor: `(varlık kimliği, 0..1 oran, kenar boşluğu)`.
#[derive(Default)]
struct ScreenLayout(Vec<(u32, Vec2, f32)>);

/// Ekran metinlerini pencerenin ŞU ANKİ boyutuna göre yeniden konumlandırır.
///
/// `WindowInfo` her yeniden boyutlandırmada tazeleniyor, ve `TextSpace::Screen` mutlak piksel
/// tuttuğu için çeviriyi birinin yapması gerekiyor. Motorun bir "oransal konum" kavramı yok —
/// bu demo o boşluğun ne kadar büyük olduğunu da göstersin diye elle yapıyor.
fn reposition(world: &mut World) {
    let Some(info) = world.get_resource::<gizmo::core::window::WindowInfo>().map(|i| *i) else {
        return;
    };
    let Some(layout) = world.get_resource::<ScreenLayout>().map(|l| l.0.clone()) else {
        return;
    };
    let mut texts = world.borrow_mut::<Text>();
    for (entity, frac, margin) in layout {
        let Some(mut text) = texts.get_mut(entity) else { continue };
        // Kenar boşluğu çapanın yönüne göre içeri doğru: solda +, sağda −, ortada 0.
        let inset = Vec2::new((0.5 - frac.x) * 2.0 * margin, (0.5 - frac.y) * 2.0 * margin);
        text.space = gizmo::renderer::components::TextSpace::Screen {
            position: Vec2::new(info.width * frac.x, info.height * frac.y) + inset,
        };
    }
}

/// Fontu bulur: `GIZMO_FONT`, sonra bilinen sistem yolları, sonra sentetik yüz.
///
/// Dönen bayrak "gerçek bir font bulundu mu" — demo bunu ekrana yazıyor, çünkü kutular gören biri
/// önce motoru suçlar.
fn load_font(renderer: &mut Renderer) -> (gizmo::renderer::text::FontId, bool) {
    if let Ok(path) = std::env::var("GIZMO_FONT") {
        match renderer.load_font_file(&path) {
            Ok(id) => {
                gizmo::gizmo_log!(Info, "font: {path} (GIZMO_FONT)");
                return (id, true);
            }
            Err(e) => gizmo::gizmo_log!(Warning, "GIZMO_FONT={path} yüklenemedi: {e}"),
        }
    }
    for path in FONT_CANDIDATES {
        if let Ok(id) = renderer.load_font_file(path) {
            gizmo::gizmo_log!(Info, "font: {path}");
            return (id, true);
        }
    }
    gizmo::gizmo_log!(
        Warning,
        "hiçbir font bulunamadı; sentetik yüz kullanılıyor (ekranda kutular görünecek)"
    );
    let id = renderer
        .load_font(gizmo::renderer::text::synthetic::synthetic_face())
        .expect("sentetik yüz her zaman ayrıştırılır");
    (id, false)
}
