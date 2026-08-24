//! # Arayüz yerleşimi — ve `gizmo-ui`'nin ilk kez bir şey çizdiği yer
//!
//! `gizmo-ui` yazıldığı günden 2026-08-24'e kadar **hiçbir vertex üretmedi**. Kendi crate
//! belgesinde yazıyordu: *"This crate emits no vertices and no draw calls"*, ve
//! `BackgroundColor` *"is written and never read"* — paketleri onu takıyordu, atölyedeki hiçbir
//! crate okumuyordu. Kutuları hesaplayan ama çizemeyen bir arayüz katmanıydı.
//!
//! Bu demo o iki cümlenin artık yanlış olduğu yer. Değişen tek şey **köprü**: `Node` mutlak
//! pencere-piksel dikdörtgeni yayınlıyordu, `TextSpace::Screen` mutlak pencere pikseli alıyordu,
//! ve arada kimse yoktu. Köprü cephede (`gizmo::systems::render::record_text`), çünkü `gizmo-ui`
//! `gizmo-app`'in ÜSTÜNDE — render katmanı bir `Node` göremez, hiç göremeyecek.
//!
//! ## Ne görüyorsunuz
//!
//! Üç düğmeli bir satır: taffy yerleşimi (`Style` → `Node`), her düğmede bir dolgu
//! (`BackgroundColor`) ve bir etiket (`Text`). Fareyi üzerine götürün — `Interaction` durumu
//! değişiyor ve dolgu onunla değişiyor. Etiketler düğmenin kendi kutusunda duruyor: `Text`'in
//! **çapası** kutunun hangi köşesine oturacağını seçiyor, `Text`'in kendi konumu yok sayılıyor.
//!
//! ## Hâlâ olmayanlar — ve demo bunları gizlemiyor
//!
//! | eksik | demoda nasıl görünüyor |
//! |-------|------------------------|
//! | **kırpma yok** | dördüncü düğmenin etiketi kutusunun sağından taşıyor, kesilmiyor |
//! | **z-sırası yok** | sıra eleman başına değil GLOBAL: önce bütün dolgular, sonra bütün glifler. Yani bir etiket her zaman her dolgunun üstünde — burada işe yarıyor, ama üst üste binen iki panelde yanlış olur, ve `gizmo-ui`'nin `Node`'unda sıralanacak bir z yok |
//! | **tıklama olayı yok** | `Interaction` kare başına yeniden hesaplanan bir DURUM, olay akışı değil — "tıklandı" ancak iki karenin farkından çıkarılır |
//! | **klavye/odak yok** | hiçbir düğmeye Tab ile gidilemez |
//!
//! ## Kontroller
//!   * **Fare** — düğmelerin üzerinde gezin, sol tuşla basılı tutun
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::hierarchy::HierarchyExt;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::ui::{
    AlignItems, BackgroundColor, ButtonBundle, Interaction, JustifyContent, NodeBundle, Style,
    UiPlugin, UiRect, Val,
};

/// Font aranacak bilinen yollar — `text` demosuyla aynı sıra, aynı sebep (bkz. oradaki başlık).
const FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    "C:/Windows/Fonts/arial.ttf",
];

/// Düğmenin durumuna göre dolgu rengi. Tek yerde, çünkü hem kurulum hem güncelleme kullanıyor.
fn fill(interaction: Interaction) -> Vec4 {
    match interaction {
        Interaction::Pressed => Vec4::new(0.22, 0.55, 0.85, 1.0),
        Interaction::Hovered => Vec4::new(0.34, 0.36, 0.42, 1.0),
        _ => Vec4::new(0.20, 0.21, 0.25, 1.0),
    }
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - UI Layout", 1280, 720)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, state| {
            let font = load_font(scene.renderer);

            // ── Arkada bir sahne olsun ki arayüzün gerçekten ÜSTTE olduğu görünsün ──────
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            for (x, c) in [
                (-2.5_f32, Vec4::new(0.75, 0.35, 0.32, 1.0)),
                (0.0, Vec4::new(0.35, 0.65, 0.42, 1.0)),
                (2.5, Vec4::new(0.35, 0.45, 0.75, 1.0)),
            ] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)),
                    GlobalTransform::default(),
                    AssetManager::create_cube(&scene.renderer.device),
                    Material::new(white.clone()).with_pbr(c, 0.6, 0.0),
                    MeshRenderer::new(),
                ));
            }

            // ── Arayüz kökü: ekranın üst şeridinde bir flex satır ────────────────────────
            let root = scene.world.spawn_bundle(NodeBundle {
                style: Style {
                    width: Val::Percent(100.0),
                    height: Val::Px(96.0),
                    padding: UiRect::all(Val::Px(16.0)),
                    column_gap: Val::Px(16.0),
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::FlexStart),
                    ..Default::default()
                },
                // Yarı saydam bir şerit: arkasındaki sahne görünüyor, yani dolgu gerçekten
                // harmanlanıyor — opak bir kutu bunu söyleyemezdi.
                background_color: BackgroundColor(Vec4::new(0.08, 0.09, 0.12, 0.75)),
                ..Default::default()
            });

            // Üç düğme, artı bilerek dar olan bir dördüncü: etiketi taşacak, çünkü kırpma yok.
            for (label, width, anchor) in [
                ("BASLAT", 150.0, TextAnchor::Center),
                ("AYARLAR", 150.0, TextAnchor::Center),
                ("CIKIS", 150.0, TextAnchor::Center),
                ("TASAN ETIKET", 70.0, TextAnchor::CenterLeft),
            ] {
                let button = scene.world.spawn_bundle(ButtonBundle {
                    style: Style {
                        width: Val::Px(width),
                        height: Val::Px(48.0),
                        ..Default::default()
                    },
                    background_color: BackgroundColor(fill(Interaction::None)),
                    ..Default::default()
                });
                // Etiketi düğmenin KENDİSİNE takıyoruz. Aynı varlıkta `Node` + `Text` olması
                // "yazı bu kutuda dursun" demek; `Text`'in kendi konumu (burada sıfır) yok
                // sayılıyor, çapası kutunun hangi noktasına oturacağını seçiyor.
                scene.world.add_component(
                    button,
                    Text::screen(label, font, 20.0, Vec2::ZERO)
                        .with_anchor(anchor)
                        .with_color(Vec4::new(0.94, 0.95, 0.97, 1.0)),
                );
                scene.world.add_child(root, button);
            }

            scene.spawn_camera(state, Vec3::new(0.0, 1.5, 8.0), Vec3::ZERO);
        })
        // `with_simple_scene` uçan kamerayı kurar ve `set_update` onu DEĞİŞTİRİR — bu yüzden
        // `simple_scene_update`'i elle çağırıyoruz (tam olarak bunun için `pub`).
        .set_update(|world, state, dt, input| {
            gizmo::simple::simple_scene_update(world, state, dt, input);
            recolor_buttons(world);
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Düğmelerin dolgusunu `Interaction` durumuna göre günceller.
///
/// `Interaction` bir OLAY değil, kare başına yeniden hesaplanan bir durum — bu yüzden burada
/// "tıklandı" diye bir dal yok: basılı tutmak `Pressed`, bırakmak doğrudan `None` ya da
/// `Hovered`. Tıklamayı ayırt etmek isteyen iki karenin farkını kendisi tutmak zorunda, ve
/// `gizmo-ui`'nin belgeleri bunu eksik olarak sayıyor.
fn recolor_buttons(world: &mut World) {
    let states: Vec<(u32, Interaction)> = {
        let interactions = world.borrow::<Interaction>();
        interactions.iter().map(|(id, i)| (id, *i)).collect()
    };
    let mut colors = world.borrow_mut::<BackgroundColor>();
    for (id, interaction) in states {
        if let Some(mut color) = colors.get_mut(id) {
            color.0 = fill(interaction);
        }
    }
}

/// Fontu bulur; hiçbiri yoksa motorun sentetik yüzüne düşer (ekranda kutular görünür).
fn load_font(renderer: &mut Renderer) -> gizmo::renderer::text::FontId {
    for path in FONT_CANDIDATES {
        if let Ok(id) = renderer.load_font_file(path) {
            gizmo::gizmo_log!(Info, "font: {path}");
            return id;
        }
    }
    gizmo::gizmo_log!(
        Warning,
        "hiçbir font bulunamadı; sentetik yüz kullanılıyor (etiketler kutu görünecek)"
    );
    renderer
        .load_font(gizmo::renderer::text::synthetic::synthetic_face())
        .expect("sentetik yüz her zaman ayrıştırılır")
}
