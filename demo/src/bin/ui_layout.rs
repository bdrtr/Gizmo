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
//! ## Artık olanlar
//!
//! - **Kaydırma** (2026-11-02): soldaki liste `Scroll` taşıyor — on iki satır, sığdığı kutu
//!   altısına yetiyor. Fare tekerleği üstündeyken kaydırıyor, kutunun dışına kayan satırlar
//!   kırpılıyor ve tıklanmıyor (görünmeyen bir düğme basılmaz). Sağ kenardaki kaydırma çubuğu
//!   (2026-11-03) sürüklenebiliyor — tekerleği olmayan bir touchpad ya da parmak için. Önceden `ClipContent` uzun bir
//!   listeyi kesebiliyordu ama geri kalanına ulaşmanın yolu yoktu. (Klavyeyle gezinme de var:
//!   `UiNavigation`, `menu` demosunda.)
//! - **Katman sırası** (2026-11-02): `ZIndex` taşıyan bir kutu (ve onu devralan torunları)
//!   daha alttakilerin hem dolgusunun hem etiketinin üstünde çiziliyor, ve fareyi de o alıyor.
//!   Düğmelerin üstüne binen "Pencere" `ZIndex(1)`: altındaki etiketler içinden
//!   görünmüyor, altındaki düğmeye tıklanmıyor. Önceden her etiket her dolgunun üstündeydi.
//! - **Kırpma** (2026-11-02): `ClipContent` taşıyan bir kutu kendi etiketini ve torunlarınınkini
//!   kendi sınırına kırpıyor. Dördüncü düğme bilerek dar: etiketi kenarında kesiliyor (önceden
//!   kutunun dışına taşıyordu).
//! - **Tıklama** (2026-11-02): `ButtonBundle`'daki `Clicked`, düğmenin üzerinde basılıp üzerinde
//!   bırakıldığı tek karede `true` — dışarıda bırakmak iptal. Düğmeler tıklandıkça sayıyor.
//!   Önceden "tıklandı" iki karenin farkından elle çıkarılıyordu.
//!
//! ## Kontroller
//!   * **Fare** — düğmelerin üzerinde gezin, sol tuşla basılı tutun
//!   * **Tekerlek** — soldaki listeyi kaydır
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::hierarchy::HierarchyExt;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::ui::{
    AlignItems, BackgroundColor, ButtonBundle, Clicked, ClipContent, FlexDirection, Interaction, JustifyContent,
    NodeBundle, Scroll, Style, UiPlugin, UiRect, Val,
};

/// Bir düğmenin etiketi ve kaç kez tıklandığı.
#[derive(Clone)]
struct Counter {
    label: &'static str,
    clicks: u32,
}
gizmo::core::impl_component!(Counter);

/// Kaydırılan listenin satırları.
const ROWS: [&str; 12] = [
    "Kirmizi kup", "Yesil kup", "Mavi kup", "Isik", "Kamera", "Zemin",
    "Ses", "Parcacik", "Yazi", "Pencere", "Liste", "Son satir",
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

            // Üç düğme, artı bilerek dar olan bir dördüncü: etiketi kutusunun kenarında kesiliyor
            // (`ClipContent`).
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
                scene.world.add_component(button, Counter { label, clicks: 0 });
                scene.world.add_component(button, ClipContent);
                scene.world.add_child(root, button);
            }

            // Düğmelerin bir kısmının üstüne binen bir pencere, bir katman yukarıda.
            let window = scene.world.spawn_bundle(NodeBundle {
                style: Style {
                    position_type: gizmo::ui::PositionType::Absolute,
                    inset: UiRect { left: Val::Px(300.0), top: Val::Px(40.0), right: Val::Auto, bottom: Val::Auto },
                    width: Val::Px(260.0),
                    height: Val::Px(140.0),
                    ..Default::default()
                },
                background_color: BackgroundColor(Vec4::new(0.18, 0.22, 0.3, 1.0)),
                ..Default::default()
            });
            scene.world.add_component(window, gizmo::ui::ZIndex(1));
            scene.world.add_component(
                window,
                Text::screen("Pencere\n(ZIndex 1)", font, 22.0, Vec2::ZERO).with_anchor(TextAnchor::Center),
            );

            // Kutusundan uzun bir liste: on iki satır, altısı görünüyor, tekerlek gerisini getiriyor.
            let list = scene.world.spawn_bundle(NodeBundle {
                style: Style {
                    position_type: gizmo::ui::PositionType::Absolute,
                    inset: UiRect { left: Val::Px(16.0), top: Val::Px(120.0), right: Val::Auto, bottom: Val::Auto },
                    width: Val::Px(220.0),
                    height: Val::Px(6.0 * 40.0 + 8.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(4.0)),
                    row_gap: Val::Px(4.0),
                    ..Default::default()
                },
                background_color: BackgroundColor(Vec4::new(0.08, 0.09, 0.12, 0.85)),
                ..Default::default()
            });
            scene.world.add_component(list, Scroll::default());
            for (i, label) in ROWS.iter().enumerate() {
                let row = scene.world.spawn_bundle(ButtonBundle {
                    style: Style { width: Val::Percent(100.0), height: Val::Px(36.0), ..Default::default() },
                    background_color: BackgroundColor(fill(Interaction::None)),
                    ..Default::default()
                });
                scene.world.add_component(
                    row,
                    Text::screen(format!("{:>2}. {label}", i + 1), font, 18.0, Vec2::ZERO)
                        .with_anchor(TextAnchor::CenterLeft)
                        .with_color(Vec4::new(0.94, 0.95, 0.97, 1.0)),
                );
                scene.world.add_component(row, Counter { label, clicks: 0 });
                scene.world.add_child(list, row);
            }

            scene.spawn_camera(state, Vec3::new(0.0, 1.5, 8.0), Vec3::ZERO);
        })
        // `with_simple_scene` uçan kamerayı kurar ve `set_update` onu DEĞİŞTİRİR — bu yüzden
        // `simple_scene_update`'i elle çağırıyoruz (tam olarak bunun için `pub`).
        .replace_update(|world, state, dt, input| {
            gizmo::simple::simple_scene_update(world, state, dt, input);
            recolor_buttons(world);
            count_clicks(world);
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
    let states: Vec<(Entity, Interaction)> = {
        world.query::<(Entity, &Interaction)>().iter().map(|(entity, i)| (entity, *i)).collect()
    };
    let mut colors = world.borrow_mut::<BackgroundColor>();
    for (id, interaction) in states {
        if let Some(mut color) = colors.get_mut(id) {
            color.0 = fill(interaction);
        }
    }
}

/// Tıklanan düğmenin sayacını artırır ve etiketine yazar. `Clicked` tam bir kare `true`.
fn count_clicks(world: &mut World) {
    let hits: Vec<Entity> = world.query::<(Entity, &Clicked)>().iter().filter(|(_, c)| c.0).map(|(entity, _)| entity).collect();
    for id in hits {
        let text = {
            let mut counters = world.borrow_mut::<Counter>();
            let Some(mut c) = counters.get_mut(id) else { continue };
            c.clicks += 1;
            format!("{} ({})", c.label, c.clicks)
        };
        if let Some(mut t) = world.borrow_mut::<Text>().get_mut(id) {
            t.content = text;
        }
    }
}

/// A font the machine has (`GIZMO_FONT` or the platform's usual faces); without one, the
/// synthetic test face (labels show as boxes).
fn load_font(renderer: &mut Renderer) -> gizmo::renderer::text::FontId {
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
